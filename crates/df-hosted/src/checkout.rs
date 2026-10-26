//! Hosted checkout persists receipts; only verified processor webhooks settle Stripe purchases.

use std::sync::Arc;

use df_account_app::wire::{CheckoutCreated, CheckoutStatus, CheckoutView, CreditOffers};
use df_portal::app_api::ports::{CheckoutPort, PortError, PortResult};
use df_types::{AccountId, CheckoutId, OperationId};

use crate::{config::Profile, database::Connection, finance::Finance};

#[allow(
    dead_code,
    reason = "PayPal checkout is built and tested; the runtime mounts it when PayPal top-up is enabled"
)]
mod paypal;
#[allow(
    dead_code,
    reason = "PayPal checkout is built and tested; the runtime mounts it when PayPal top-up is enabled"
)]
mod paypal_provider;
#[allow(
    dead_code,
    reason = "PayPal checkout is built and tested; the runtime mounts it when PayPal top-up is enabled"
)]
mod paypal_settings;
mod paypal_webhook;
mod provider;
mod settings;
mod stripe;
mod webhook;

pub(crate) use paypal_settings::Settings as PaypalSettings;
pub(crate) use settings::Settings;

pub(crate) fn router(checkout: Checkout) -> axum::Router {
    webhook::router(checkout.clone()).merge(paypal_webhook::router(checkout))
}

#[derive(Clone)]
pub(crate) struct Checkout {
    development: Option<Arc<tokio::sync::Mutex<Connection>>>,
    stripe: Option<Arc<stripe::Stripe>>,
    paypal: Option<Arc<paypal::Paypal>>,
    finance: Finance,
}

impl Checkout {
    pub(crate) async fn configured(
        profile: Profile,
        connection: Connection,
        finance: Finance,
        settings: Option<Settings>,
        origin: &str,
    ) -> PortResult<Self> {
        if let Some(settings) = settings {
            let provider = provider::Provider::new(settings, origin).map_err(|error| {
                error.audit();
                PortError::Unavailable
            })?;
            return Ok(Self {
                development: None,
                paypal: None,
                stripe: Some(Arc::new(stripe::Stripe {
                    connection: tokio::sync::Mutex::new(connection),
                    provider,
                    creations: Arc::new(std::sync::Mutex::new(stripe::CreationState::default())),
                })),
                finance,
            });
        }
        let development = if profile == Profile::Development {
            connection.client.batch_execute(
                "CREATE TABLE IF NOT EXISTS hosted.development_checkout (\
                 checkout_id bytea PRIMARY KEY, account_id bytea NOT NULL, operation_id bytea NOT NULL, \
                 amount_cents integer NOT NULL CHECK(amount_cents > 0), paid boolean NOT NULL DEFAULT false, \
                 UNIQUE(account_id, operation_id))",
            ).await.map_err(|_| PortError::Unavailable)?;
            Some(Arc::new(tokio::sync::Mutex::new(connection)))
        } else {
            None
        };
        Ok(Self {
            development,
            stripe: None,
            paypal: None,
            finance,
        })
    }

    #[allow(
        dead_code,
        reason = "PayPal checkout is built and tested; the runtime mounts it when PayPal top-up is enabled"
    )]
    pub(crate) fn with_paypal(
        mut self,
        connection: Connection,
        settings: Option<PaypalSettings>,
        origin: &str,
    ) -> PortResult<Self> {
        if let Some(settings) = settings {
            let provider = paypal_provider::Provider::new(settings, origin).map_err(|error| {
                error.audit_for("paypal");
                PortError::Unavailable
            })?;
            let creations = self.stripe.as_ref().map_or_else(
                || Arc::new(std::sync::Mutex::new(stripe::CreationState::default())),
                |stripe| Arc::clone(&stripe.creations),
            );
            self.development = None;
            self.paypal = Some(Arc::new(paypal::Paypal {
                connection: tokio::sync::Mutex::new(connection),
                provider,
                creations,
            }));
        }
        Ok(self)
    }

    pub(crate) async fn complete(
        &self,
        account: AccountId,
        checkout: CheckoutId,
    ) -> PortResult<()> {
        let connection = self
            .development
            .as_ref()
            .ok_or(PortError::Unavailable)?
            .lock()
            .await;
        let row = connection.client.query_opt(
            "SELECT amount_cents FROM hosted.development_checkout WHERE account_id = $1 AND checkout_id = $2",
            &[&&account.as_bytes()[..], &&checkout.as_bytes()[..]],
        ).await.map_err(|_| PortError::Unavailable)?.ok_or(PortError::NotFound)?;
        let cents: i32 = row.try_get(0).map_err(|_| PortError::Unavailable)?;
        let cents = u32::try_from(cents).map_err(|_| PortError::Unavailable)?;
        let reference = df_commerce::finance_ids::hex(checkout.as_bytes());
        // A lost acknowledgement is safely replayed under the finance processor/payment key.
        self.finance.purchase(account, cents, &reference).await?;
        connection.client.execute(
            "UPDATE hosted.development_checkout SET paid = true WHERE account_id = $1 AND checkout_id = $2",
            &[&&account.as_bytes()[..], &&checkout.as_bytes()[..]],
        ).await.map_err(|_| PortError::UnknownOutcome)?;
        Ok(())
    }
}

impl CheckoutPort for Checkout {
    fn offers(&self) -> CreditOffers {
        CreditOffers {
            is_test_mode: self
                .stripe
                .as_ref()
                .is_none_or(|stripe| stripe.provider.settings.is_test_mode)
                && self
                    .paypal
                    .as_ref()
                    .is_none_or(|paypal| paypal.provider.settings.is_test_mode),
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
        let offers = self.offers();
        if !(offers.custom_minimum_cents..=offers.custom_maximum_cents).contains(&amount_cents) {
            return Err(PortError::Forbidden);
        }
        if let Some(stripe) = &self.stripe {
            return stripe.create(account, amount_cents, operation).await;
        }
        let Some(connection) = &self.development else {
            df_observe::record_audit_line(
                "hosted.payments_not_configured",
                "HOSTED payments_not_configured".to_owned(),
            );
            return Err(PortError::Unavailable);
        };
        let connection = connection.lock().await;
        let token = df_customer::SecretToken::generate().map_err(|_| PortError::Unavailable)?;
        let digest =
            df_commerce::finance_ids::unhex(&token.text()).ok_or(PortError::Unavailable)?;
        let id = digest.get(..16).ok_or(PortError::Unavailable)?;
        let cents = i32::try_from(amount_cents).map_err(|_| PortError::Unavailable)?;
        let row = connection.client.query_one(
            "INSERT INTO hosted.development_checkout(checkout_id, account_id, operation_id, amount_cents) \
             VALUES($1,$2,$3,$4) ON CONFLICT(account_id,operation_id) DO UPDATE SET operation_id = excluded.operation_id \
             RETURNING checkout_id, amount_cents",
            &[&id, &&account.as_bytes()[..], &&operation.as_bytes()[..], &cents],
        ).await.map_err(|_| PortError::UnknownOutcome)?;
        let original: i32 = row.try_get(1).map_err(|_| PortError::Unavailable)?;
        if original != cents {
            return Err(PortError::Forbidden);
        }
        let id: Vec<u8> = row.try_get(0).map_err(|_| PortError::Unavailable)?;
        let id = df_commerce::finance_ids::hex(&id);
        Ok(CheckoutCreated {
            checkout_id: id.clone(),
            redirect_url: format!("/app/dev/checkouts/{id}"),
        })
    }

    async fn status(&self, account: AccountId, checkout: CheckoutId) -> PortResult<CheckoutView> {
        if let Some(paypal) = &self.paypal {
            match paypal.status(account, checkout).await {
                Err(PortError::NotFound) => {}
                result => return result,
            }
        }
        if let Some(stripe) = &self.stripe {
            return stripe.status(account, checkout).await;
        }
        if self.paypal.is_some() {
            return Err(PortError::NotFound);
        }
        let connection = self
            .development
            .as_ref()
            .ok_or(PortError::Unavailable)?
            .lock()
            .await;
        let row = connection.client.query_opt(
            "SELECT amount_cents, paid FROM hosted.development_checkout WHERE account_id = $1 AND checkout_id = $2",
            &[&&account.as_bytes()[..], &&checkout.as_bytes()[..]],
        ).await.map_err(|_| PortError::Unavailable)?.ok_or(PortError::NotFound)?;
        let cents: i32 = row.try_get(0).map_err(|_| PortError::Unavailable)?;
        let paid: bool = row.try_get(1).map_err(|_| PortError::Unavailable)?;
        Ok(CheckoutView {
            checkout_id: df_commerce::finance_ids::hex(checkout.as_bytes()),
            amount_cents: u32::try_from(cents).map_err(|_| PortError::Unavailable)?,
            status: if paid {
                CheckoutStatus::Paid
            } else {
                CheckoutStatus::Pending
            },
        })
    }
}

#[cfg(test)]
mod tests;
